#!/usr/bin/env node
// Synthetic `claude` in stream-json mode only. Never launches a shell, model or
// network request. Each user message names a scenario; the client answers the
// way Claude Code does, so the bridge and the Rust adapter can be compared.
import { createInterface } from 'node:readline';
import { appendFileSync } from 'node:fs';
// The parity suite reads what the client received; run by hand, nothing is logged.
const log=value=>{if(process.env.TENDLY_CHAT_TEST_LOG)appendFileSync(process.env.TENDLY_CHAT_TEST_LOG,JSON.stringify(value)+'\n');};
const send=message=>process.stdout.write(JSON.stringify(message)+'\n');
log({event:'args',args:process.argv.slice(2)});
const session='claude-fixture-session';
let turn=0,mode='',holding=false,pending,asked=0;

const success=(request,response={})=>send({type:'control_response',response:{subtype:'success',request_id:request.request_id,response}});
const refuse=(request,error)=>send({type:'control_response',response:{subtype:'error',request_id:request.request_id,error}});
function reply(text='Hello 🦊'){
  const id='msg-'+turn;
  send({type:'stream_event',event:{type:'message_start',message:{id}}});
  send({type:'stream_event',event:{type:'content_block_delta',delta:{type:'text_delta',text:'Hello '}}});
  send({type:'assistant',message:{id,content:[{type:'text',text}],usage:{input_tokens:12,cache_read_input_tokens:30,cache_creation_input_tokens:2,output_tokens:4}}});
}
function result(subtype='success',extra={}){
  send({type:'result',subtype,is_error:subtype!=='success',result:'Hello 🦊',usage:{input_tokens:12,cache_read_input_tokens:30,output_tokens:4},modelUsage:{'claude-fixture':{contextWindow:200000}},...extra});
}
const done=()=>{reply();result();};
const permission=(name,input,extra={})=>{
  pending={request_id:'approval-'+(++asked),request:{subtype:'can_use_tool',tool_name:name,tool_use_id:'tool-'+turn,input,...extra}};
  send({type:'assistant',message:{id:'tools-'+turn,content:[{type:'tool_use',id:'tool-'+turn,name,input}]}});
  send({type:'control_request',...pending});
};

function begin(text){
  turn++;mode=text;
  send({type:'system',subtype:'init',session_id:session,model:'claude-sonnet-5-fixture',slash_commands:['review','compact']});
  switch(text){
    case 'hold':holding=true;send({type:'tool_progress',tool_use_id:'hold-'+turn,tool_name:'Fixture wait'});return;
    case 'tools':
      send({type:'stream_event',event:{type:'content_block_start',content_block:{type:'tool_use',id:'read-'+turn,name:'Read',input:{}}}});
      send({type:'assistant',message:{id:'tools-'+turn,content:[{type:'text',text:'Looking.'},{type:'tool_use',id:'read-'+turn,name:'Read',input:{file_path:'src/main.rs'}},{type:'tool_use',id:'bash-'+turn,name:'Bash',input:{command:'ls'}}],usage:{input_tokens:40,output_tokens:2}}});
      send({type:'tool_progress',tool_use_id:'bash-'+turn,tool_name:'Bash'});
      send({type:'user',message:{content:[{type:'tool_result',tool_use_id:'read-'+turn,content:'fn main() {}'},{type:'tool_result',tool_use_id:'bash-'+turn,is_error:true,content:[{type:'text',text:'ls: denied'},{type:'image'}]}]}});
      done();return;
    case 'approve':
      return permission('Bash',{command:'rm -rf /tmp/fixture-outside',description:'Remove fixture output'},{decision_reason:'Bash is not allowed in this mode',blocked_path:'/tmp/fixture-outside',title:'Allow Bash to remove output?',permission_suggestions:[{type:'addRules',rules:[{toolName:'Bash'}],behavior:'allow',destination:'localSettings'},'not an update',{rules:[]}]});
    case 'write':return permission('Write',{file_path:'notes.md',content:'hi'});
    case 'ask':
      return permission('AskUserQuestion',{questions:[{header:'Format',question:'Which format?',multiSelect:false,options:[{label:'Brief',description:'One line'},{label:'Long'}]},{question:'Which parts?',multiSelect:true,options:[{label:'API'},{label:'UI'}]}]},{requires_user_interaction:true});
    case 'ask-invalid':return permission('AskUserQuestion',{questions:[{question:'Same?'},{question:'Same?'}]});
    case 'interaction':return permission('Bash',{command:'login'},{requires_user_interaction:true});
    case 'unsupported':pending={request_id:'other-'+(++asked),request:{subtype:'elicitation',server:'fixture'}};send({type:'control_request',...pending});return;
    case 'dialog':holding=true;send({type:'control_request',request_id:'dialog-'+(++asked),request:{subtype:'request_user_dialog',dialog:'login'}});return;
    case 'withdrawn':
      permission('Edit',{file_path:'a.txt'});
      send({type:'control_cancel_request',request_id:pending.request_id});pending=undefined;done();return;
    case 'subagent':
      send({type:'assistant',message:{id:'lead-'+turn,content:[{type:'tool_use',id:'task-'+turn,name:'Task',input:{description:'Survey the code',prompt:'Look around'}}]}});
      send({type:'system',subtype:'task_started',task_id:'t-'+turn,tool_use_id:'task-'+turn,description:'Survey the code',task_type:'local_agent',is_backgrounded:true});
      send({type:'assistant',parent_tool_use_id:'task-'+turn,message:{content:[{type:'text',text:'  Reading the tree  '},{type:'tool_use',id:'grep-'+turn,name:'Grep',input:{pattern:'TODO',path:'/repo/src/lib.rs'}}]}});
      send({type:'user',parent_tool_use_id:'task-'+turn,message:{content:[{type:'tool_result',tool_use_id:'grep-'+turn,content:'lib.rs:3: TODO'}]}});
      send({type:'system',subtype:'task_progress',task_id:'t-'+turn,summary:'Searching',usage:{total_tokens:900,tool_uses:1,duration_ms:1200}});
      send({type:'system',subtype:'task_progress',task_id:'t-'+turn,summary:'Searching'});
      send({type:'system',subtype:'task_progress',task_id:'t-'+turn,summary:'Reading'});
      send({type:'system',subtype:'task_updated',task_id:'t-'+turn,patch:{is_backgrounded:true}});
      send({type:'user',message:{content:[{type:'tool_result',tool_use_id:'task-'+turn,content:'Started in the background.'}]}});
      done();
      // The task ends after the turn that started it.
      send({type:'system',subtype:'task_notification',task_id:'t-'+turn,status:'completed',summary:'Found 1 TODO',usage:{total_tokens:2000,tool_uses:3,duration_ms:5000}});
      send({type:'system',subtype:'task_started',task_id:'sh-'+turn,tool_use_id:'task-'+turn,task_type:'local_bash',description:'npm test'});
      send({type:'system',subtype:'task_notification',task_id:'sh-'+turn,status:'failed'});
      return;
    case 'background':
      done();
      // A background command it waited on finished: it carries on by itself.
      reply('Carried on');result();
      return;
    case 'fail-auth':reply();result('error_during_execution',{errors:['OAuth token is not supported for this endpoint']});return;
    case 'fail':result('error_max_turns',{result:''});return;
    // What the client sends when the endpoint refuses a request: a synthetic
    // assistant message, then a result that still says `success`.
    case 'fail-401':{
      const said='Failed to authenticate. API Error: 401 bad key';
      send({type:'assistant',error:'authentication_failed',is_api_error_message:true,api_error_status:401,message:{id:'error-'+turn,model:'<synthetic>',content:[{type:'text',text:said}]}});
      result('success',{is_error:true,api_error_status:401,terminal_reason:'api_error',result:said});return;
    }
    case 'result-only':result();return;
    // Load scenarios for the long-conversation harness: a full reply with
    // the parts that cost the most to render, and a reply streamed at speed.
    case 'seed':{
      const code=Array.from({length:30},(_,i)=>`  let value_${i} = compute(${i}); // step ${i}`).join('\n');
      const text=`## Turn ${turn}\n\nHere is what changed:\n\n- the parser now keeps its place\n- **the cache** is bounded\n- \`compact_deltas\` runs once\n\n\`\`\`rust\nfn main() {\n${code}\n}\n\`\`\`\n\n| file | lines |\n| --- | --- |\n| a.rs | 120 |\n| b.rs | 48 |\n\nThat is all for turn ${turn}.`;
      send({type:'assistant',message:{id:'tools-'+turn,content:[{type:'tool_use',id:'read-'+turn,name:'Read',input:{file_path:'src/lib.rs'}},{type:'tool_use',id:'bash-'+turn,name:'Bash',input:{command:'cargo test'}}]}});
      send({type:'user',message:{content:[{type:'tool_result',tool_use_id:'read-'+turn,content:'fn lib() {}\n'.repeat(40)},{type:'tool_result',tool_use_id:'bash-'+turn,content:'test result: ok. 12 passed'}]}});
      reply(text);result();return;
    }
    case 'mode-change':send({type:'system',subtype:'status',permissionMode:'acceptEdits'});done();return;
    default:
      if(text.startsWith('stream:')){
        const pieces=Number(text.slice(7))||200,id='msg-'+turn;let sent=0;
        send({type:'stream_event',event:{type:'message_start',message:{id}}});
        const timer=setInterval(()=>{
          send({type:'stream_event',event:{type:'content_block_delta',delta:{type:'text_delta',text:sent%40===39?'\n\n':`word${sent} `}}});
          if(++sent>=pieces){clearInterval(timer);reply('streamed '+pieces+' pieces');result();}
        },15);
        return;
      }
      done();
  }
}

for await(const line of createInterface({input:process.stdin})){
  const message=JSON.parse(line);log(message);
  if(message.type==='control_request'){
    const request=message.request;
    if(request.subtype==='initialize'){success(message,{commands:[{name:'review',description:'Review'},'compact',{nameless:true}],models:[{value:'default',displayName:'Default (Sonnet)'},{value:'sonnet',displayName:'Sonnet',supportsEffort:true,supportedEffortLevels:['low','medium','high']},{value:'opus',displayName:'Opus',description:'Fixture opus'},{displayName:'no value'}]});continue;}
    if(request.subtype==='set_model'){if(request.model==='refused-model')refuse(message,'API error: 400 availability probe refused');else success(message);continue;}
    // The status that follows comes a moment later: written in the same chunk,
    // the bridge reads it before its awaited answer and reports the mode twice.
    if(request.subtype==='set_permission_mode'){success(message,{mode:request.mode});setTimeout(()=>send({type:'system',subtype:'status',permissionMode:request.mode}),50);continue;}
    if(request.subtype==='interrupt'){success(message);if(holding){holding=false;result('error_during_execution');}continue;}
    refuse(message,'Unknown control request');continue;
  }
  if(message.type==='control_response'){
    const answer=message.response;
    if(!pending||answer.request_id!==pending.request_id)continue;
    const asking=pending;pending=undefined;
    if(answer.subtype==='error'){result();continue;}
    const behavior=answer.response.behavior;
    send({type:'user',message:{content:[{type:'tool_result',tool_use_id:asking.request.tool_use_id,is_error:behavior==='deny',content:behavior==='deny'?answer.response.message:'done'}]}});
    if(answer.response.interrupt){result('error_during_execution');continue;}
    done();continue;
  }
  if(message.type==='user'){
    const text=message.message.content;
    const replay=()=>send({type:'user',isReplay:true,uuid:message.uuid,message:message.message});
    // Written while `hold` runs: `join` is read inside that turn, `late` only
    // after it ended, so it runs as a turn of its own.
    if(holding&&(text==='join'||text==='late')){
      holding=false;
      if(text==='join'){replay();done();}
      else{done();turn++;replay();done();}
      continue;
    }
    replay();begin(text);continue;
  }
}
