#!/usr/bin/env node
// Synthetic stdio protocol only. Never launches a shell, native agent, network request or model.
import { createInterface } from 'node:readline';
import { appendFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const codex=process.argv.includes('app-server');
const log=value=>appendFileSync(process.env.TENDLY_CHAT_TEST_LOG,JSON.stringify(value)+'\n');
const send=message=>process.stdout.write(JSON.stringify(message)+'\n');
const native=(method,params)=>send({method,params});
let turn=0,thread='fixture-thread',mode='',activeTurn,requestId,started=0;
assert.equal(process.env.TENDLY_CHAT_TEST_INHERITED,'kept');
process.stderr.write(process.env.TENDLY_SECRET_TEST_CREDENTIAL+' diagnostic must not reach the host\n');
log({event:'args',args:process.argv.slice(2)});

function completed(status='completed') {
  if(codex){
    native('item/agentMessage/delta',{threadId:thread,turnId:activeTurn,itemId:'assistant-'+turn,delta:'Hello '});
    const line=Buffer.from(JSON.stringify({method:'item/agentMessage/delta',params:{threadId:thread,turnId:activeTurn,itemId:'assistant-'+turn,delta:'🦊'}})+'\n');
    const split=line.indexOf(Buffer.from('🦊'))+1;process.stdout.write(line.subarray(0,split));process.stdout.write(line.subarray(split));
    native('item/completed',{threadId:thread,turnId:activeTurn,item:{id:'assistant-'+turn,type:'agentMessage',text:'Hello 🦊'}});
    native('thread/tokenUsage/updated',{threadId:thread,tokenUsage:{last:{inputTokens:12,outputTokens:4,totalTokens:16}}});
    native('turn/completed',{threadId:thread,turn:{id:activeTurn,status}});
  }else{
    send({type:'stream_event',event:{type:'message_start',message:{id:'assistant-'+turn}}});
    send({type:'stream_event',event:{type:'content_block_delta',delta:{type:'text_delta',text:'Hello 🦊'}}});
    send({type:'assistant',session_id:thread,message:{id:'assistant-'+turn,content:[{type:'text',text:'Hello 🦊'}]}});
    send({type:'result',subtype:status==='completed'?'success':'error_during_execution',is_error:status!=='completed',usage:{input_tokens:12,output_tokens:4},result:'Hello 🦊'});
  }
}
for await(const line of createInterface({input:process.stdin})) {
  const message=JSON.parse(line);log(message);
  if(codex){
    if(message.method==='initialize'){send({id:message.id,result:{}});continue;}
    if(message.method==='initialized')continue;
    // A real app-server publishes its models and the levels each one supports.
    // The real app-server answers with `data`, and marks some entries hidden.
    if(message.method==='model/list'){send({id:message.id,result:{data:[{id:'gpt-fixture',displayName:'Fixture',isDefault:true,supportedReasoningEfforts:[{reasoningEffort:'low'},{reasoningEffort:'high'}]},{id:'gpt-plain'},{id:'gpt-hidden',hidden:true}],nextCursor:null}});continue;}
    // A second thread/start is a new thread, as `/clear` asks for.
    if(message.method==='thread/start'||message.method==='thread/resume'){if(message.method==='thread/start'&&++started>1)thread='fixture-thread-'+started;else thread=message.params.threadId??thread;send({id:message.id,result:{thread:{id:thread},sandbox:{type:'readOnly'}}});continue;}
    // Steering joins the active turn; `review` is a turn that refuses it, like /review.
    if(message.method==='turn/steer'){
      if(message.params.expectedTurnId!==activeTurn||mode==='review'){send({id:message.id,error:{code:-32600,message:'This turn cannot be steered.'}});if(mode==='review'){mode='';completed();}continue;}
      send({id:message.id,result:{turnId:activeTurn}});mode='';completed();continue;
    }
    if(message.method==='turn/interrupt'){send({id:message.id,result:{}});native('turn/completed',{threadId:thread,turn:{id:activeTurn,status:'interrupted'}});continue;}
    // `slow` answers turn/start late, so whatever the client is asked meanwhile
    // arrives before it knows the turn's id; the turn then waits like `hold`.
    if(message.method==='turn/start'&&message.params.input[0].text==='slow'){
      turn++;activeTurn='turn-'+turn;mode='slow';const id=message.id,current=activeTurn,item='slow-'+turn;
      setTimeout(()=>{send({id,result:{turn:{id:current,status:'inProgress'}}});native('turn/started',{threadId:thread,turn:{id:current}});native('item/started',{threadId:thread,turnId:current,item:{id:item,type:'commandExecution',command:'fixture wait',status:'inProgress'}});},300);
      continue;
    }
    // `slowfail` refuses the turn late; `racing` runs the whole turn before
    // answering turn/start. Either way a steer written meanwhile has no turn
    // to join and must be passed on, not lost.
    if(message.method==='turn/start'&&message.params.input[0].text==='slowfail'){
      const id=message.id;setTimeout(()=>send({id,error:{code:-32603,message:'Turn could not start.'}}),300);continue;
    }
    if(message.method==='turn/start'&&message.params.input[0].text==='racing'){
      turn++;activeTurn='turn-'+turn;mode='';const id=message.id,current=activeTurn;
      setTimeout(()=>{native('turn/started',{threadId:thread,turn:{id:current}});native('turn/completed',{threadId:thread,turn:{id:current,status:'completed'}});send({id,result:{turn:{id:current,status:'inProgress'}}});},300);
      continue;
    }
    if(message.method==='turn/start'){
      turn++;activeTurn='turn-'+turn;mode=message.params.input[0].text;send({id:message.id,result:{turn:{id:activeTurn,status:'inProgress'}}});native('turn/started',{threadId:thread,turn:{id:activeTurn}});
      if(mode==='hold'||mode==='review'){native('item/started',{threadId:thread,turnId:activeTurn,item:{id:'hold-'+turn,type:'commandExecution',command:'fixture wait',status:'inProgress'}});continue;}
      if(mode==='approve'){
        native('item/started',{threadId:thread,turnId:activeTurn,item:{type:'commandExecution',id:'tool-'+turn,command:'printf fixture',status:'inProgress'}});
        requestId='approval-'+turn;send({id:requestId,method:'item/commandExecution/requestApproval',params:{threadId:thread,turnId:activeTurn,itemId:'tool-'+turn,command:'printf fixture',availableDecisions:['accept','decline','cancel']}});continue;
      }
      if(mode==='ask'){requestId='approval-'+turn;send({id:requestId,method:'item/tool/requestUserInput',params:{threadId:thread,turnId:activeTurn,questions:[{id:'format',header:'Format',question:'Which format?',options:[{label:'Brief'},{label:'Long'}],isOther:true}]}});continue;}
      if(mode==='permissions'){requestId='approval-'+turn;send({id:requestId,method:'item/permissions/requestApproval',params:{threadId:thread,turnId:activeTurn,permissions:{network:{enabled:true}}}});continue;}
      if(mode==='tools'){
        for(const item of [{id:'file-1',type:'fileChange',changes:[{path:'fixture.txt',diff:'+fixture'}]},{id:'mcp-1',type:'mcpToolCall',server:'fixture',tool:'read',arguments:{path:'fixture.txt'}}]){
          native('item/started',{threadId:thread,turnId:activeTurn,item:{...item,status:'inProgress'}});native('item/completed',{threadId:thread,turnId:activeTurn,item:{...item,status:'completed'}});
        }
      }
      if(mode==='unknown'){requestId='unknown-'+turn;send({id:requestId,method:'item/tool/call',params:{threadId:thread,turnId:activeTurn}});continue;}
      completed();continue;
    }
    if(message.id===requestId){
      if(mode==='unknown')assert.equal(message.error.code,-32601);
      else if(mode==='ask')assert.deepEqual(message.result.answers,{format:{answers:['Brief']}});
      else if(mode==='permissions'){assert.equal(message.result.scope,'turn');assert.ok(JSON.stringify(message.result.permissions)==='{}'||JSON.stringify(message.result.permissions)==='{"network":{"enabled":true}}');}
      else {assert.ok(['accept','decline','cancel'].includes(message.result.decision));native('item/completed',{threadId:thread,turnId:activeTurn,item:{type:'commandExecution',id:'tool-'+turn,command:'printf fixture',status:message.result.decision==='accept'?'completed':'declined'}});}
      native('serverRequest/resolved',{threadId:thread,requestId});completed(mode==='unknown'?'failed':message.result?.decision==='cancel'?'interrupted':'completed');continue;
    }
    throw Error('Unexpected Codex fixture input');
  }
  if(message.type==='control_request'){
    // Claude publishes its models in the initialize response and switches them
    // in place with set_model; neither starts a new native context.
    const response=message.request.subtype==='initialize'
      ? {models:[{value:'opus',displayName:'Opus',description:'Fixture opus',supportsEffort:true,supportedEffortLevels:['low','high']},{value:'haiku',displayName:'Haiku'}]}
      : {};
    // A client refusal states its own reason; the bridge must carry it through.
    if(message.request.subtype==='set_model'&&message.request.model==='refused-model'){
      send({type:'control_response',response:{subtype:'error',request_id:message.request_id,error:'API error: 400 availability probe refused'}});
      continue;
    }
    send({type:'control_response',response:{subtype:'success',request_id:message.request_id,response}});
    if(message.request.subtype==='interrupt')completed('interrupted');
    else assert.ok(['initialize','set_model'].includes(message.request.subtype));
    continue;
  }
  if(message.type==='user'){
    const replay=()=>{if(process.argv.includes('--replay-user-messages'))send({type:'user',isReplay:true,uuid:message.uuid,message:message.message});};
    // A message written while `hold` runs: `join` is read inside that turn,
    // `late` only after it has ended, so it runs as a turn of its own.
    if(mode==='hold'&&['join','late'].includes(message.message.content)){
      if(message.message.content==='join'){replay();completed();}
      else{completed();turn++;replay();completed();}
      mode='';continue;
    }
    replay();
    turn++;thread='fixture-claude';mode=message.message.content;assert.match(message.uuid,/^[a-f0-9-]{36}$/);
    send({type:'system',subtype:'init',session_id:thread});
    if(mode==='hold'){send({type:'tool_progress',tool_use_id:'hold-'+turn,tool_name:'Fixture wait'});continue;}
    if(mode==='approve'||mode==='ask'){
      requestId='approval-'+turn;
      const request=mode==='ask'?{subtype:'can_use_tool',tool_name:'AskUserQuestion',tool_use_id:'tool-'+turn,requires_user_interaction:true,input:{questions:[{header:'Format',question:'Which format?',multiSelect:false,options:[{label:'Brief'},{label:'Long'}]}]}}:{subtype:'can_use_tool',tool_name:'Bash',tool_use_id:'tool-'+turn,input:{command:'printf fixture'}};
      send({type:'assistant',message:{id:'tools-'+turn,content:[{type:'tool_use',id:'tool-'+turn,name:request.tool_name,input:request.input}]}});
      send({type:'control_request',request_id:requestId,request});continue;
    }
    if(mode==='unknown'){requestId='unknown-'+turn;send({type:'control_request',request_id:requestId,request:{subtype:'unsupported_fixture_request'}});continue;}
    completed();continue;
  }
  if(message.type==='control_response'){
    assert.equal(message.response.request_id,requestId);
    if(mode==='unknown')assert.equal(message.response.subtype,'error');
    else {
      const result=message.response.response;assert.ok(['allow','deny'].includes(result.behavior));assert.equal(result.updatedPermissions,undefined);
      if(mode==='ask'&&result.behavior==='allow')assert.deepEqual(result.updatedInput.answers,{'Which format?':'Brief'});
      send({type:'user',message:{content:[{type:'tool_result',tool_use_id:'tool-'+turn,is_error:result.behavior==='deny',content:'fixture tool result'}]}});
    }
    completed(mode==='unknown'?'failed':message.response?.response?.interrupt?'interrupted':'completed');continue;
  }
  throw Error('Unexpected Claude fixture input');
}
