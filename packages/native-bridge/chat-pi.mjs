import { ChatBase, NativeProcess, clip, nativeId } from './chat-common.mjs';

/**
 * Pi's structured mode is `pi --mode rpc`: commands in, a response per
 * command and a stream of agent events out, one JSON object per line.
 *
 * The launch flags Tendly builds for a terminal run (providers.rs) are
 * carried over, with RPC mode in front of them.
 */
export function piLaunch(job) {
  const args=['--mode','rpc'];let resume=job.resume_id;
  for(let i=0;i<job.args.length;i++) {
    const arg=job.args[i],equal=arg.indexOf('='),flag=equal>0?arg.slice(0,equal):arg;
    const value=()=>{const result=equal>0?arg.slice(equal+1):job.args[++i];if(typeof result!=='string'||!result)throw Error('Missing Pi launch option value.');return result;};
    if(['--model','--provider','--thinking','--name','-n'].includes(flag))args.push(flag,value());
    else if(flag==='--session'){const id=value();if(!nativeId(id)||(resume&&resume!==id))throw Error('Invalid or conflicting Pi session ID.');resume=id;}
    else throw Error('Unsupported Pi launch option in structured mode.');
  }
  if(resume)args.push('--session',resume);
  return {args};
}

/** A Pi command, `{id, type, ...fields}`. */
const piFrame=(id,type,fields)=>({id,type,...fields});
/** A Pi response, matched to its command by id. */
const piReply=message=>message?.type==='response'&&message.id!==undefined
  ? {id:message.id,...(message.success===false?{refused:message.error??true}:{result:message.data??{}})}
  : undefined;

/**
 * Its models as `provider/id`, the one name that runs a model of any of its
 * providers. Levels are offered only where the model reasons at all.
 */
function piModels(data) {
  const rows=Array.isArray(data?.models)?data.models:[];
  return rows.flatMap(row=>typeof row?.id==='string'&&typeof row.provider==='string'
    ? [{id:`${row.provider}/${row.id}`,name:typeof row.name==='string'&&row.name?row.name:row.id,...(row.reasoning===true?{efforts:['low','medium','high']}:{})}]
    : []);
}
const SHARED_LEVELS=['low','medium','high','xhigh','max'];
const modelName=model=>model&&typeof model.provider==='string'&&typeof model.id==='string'?`${model.provider}/${model.id}`:undefined;
const textOf=content=>typeof content==='string'?content:Array.isArray(content)?content.filter(block=>block?.type==='text').map(block=>block.text??'').join(''):'';

export class PiChat extends ChatBase {
  async initialize() {
    this.launch=piLaunch(this.job);this.tools=new Map();this.texts=new Map();this.messageCount=0;
    this.port=new NativeProcess(this.job.program,this.launch.args,this.job.cwd,message=>this.notification(message),message=>this.fatal(message),()=>this.fatal('Pi exited.'),{...this.options,frame:piFrame,reply:piReply,clientName:'Pi'});
    const state=await this.port.rpc('get_state',{});
    const commands=await this.port.rpc('get_commands',{},true).catch(()=>undefined);
    this.announce(state?.sessionId,Array.isArray(commands?.commands)?commands.commands.map(command=>command?.name):undefined);
    this.follow(state);
    // Asking for models must not hold up the session.
    void this.port.rpc('get_available_models',{},true).then(data=>{this.models=piModels(data);this.report();}).catch(()=>{});
  }
  /** What the session is set to now, from Pi's own state. */
  follow(state) {
    if(!state||typeof state!=='object')return;
    this.model=modelName(state.model)??this.model;
    this.window=Number.isSafeInteger(state.model?.contextWindow)?state.model.contextWindow:this.window;
    // Pi also has `off` and `minimal`; Tendly's depth control names only
    // the levels every client shares, so those read as no depth chosen.
    if(typeof state.thinkingLevel==='string')this.effort=SHARED_LEVELS.includes(state.thinkingLevel)?state.thinkingLevel:undefined;
    this.report();
  }
  report() { this.settings(this.models,this.model,this.effort); }
  // No approvals to switch between: Pi runs its tools without asking.
  get permissionModes(){return undefined;}
  setPermissionMode() { throw Error('Pi runs its tools without asking; it has no approval modes to switch between.'); }
  async selectModel(message) {
    if(this.active)throw Error('Wait for the current turn to finish before changing the model.');
    if(message.model){
      const cut=message.model.indexOf('/');
      if(cut<1)throw Error('Name a Pi model as provider/id.');
      const model=await this.port.rpc('set_model',{provider:message.model.slice(0,cut),modelId:message.model.slice(cut+1)});
      this.model=modelName(model)??message.model;
      if(Number.isSafeInteger(model?.contextWindow))this.window=model.contextWindow;
    }
    if(message.effort){await this.port.rpc('set_thinking_level',{level:message.effort});this.effort=message.effort;}
    // Pi clamps a level its model cannot use, and a new model brings the level
    // it allows, both without saying so: what it runs is what get_state says.
    this.follow(await this.port.rpc('get_state',{},true).catch(()=>undefined));
    this.report();
  }
  async message(message) {
    const active=this.begin(message);if(!active)return;
    try{await this.port.rpc('prompt',{message:message.content});}
    catch(error){if(this.active===active){this.error(error.message);this.finish('failed');return;}}
    // Pi answers a prompt an extension handled -- its command, or its input
    // handler -- and starts no run, so no agent_settled will end this turn.
    // A real run is already under way when Pi answers (isStreaming), so a turn
    // whose run has not started and that Pi says is not streaming is over.
    // Asked after every prompt, so the answer does not hang on whether Pi's
    // agent_start was read before its reply.
    if(this.active!==active)return;
    const state=await this.port.rpc('get_state',{},true).catch(()=>undefined);
    if(this.active===active&&!active.started&&state?.isStreaming===false)this.finish(active.interrupted?'interrupted':'completed');
  }
  /** Written while a turn runs, it is delivered before Pi's next model call. */
  async steer(message) {
    if(!this.active){await this.message(message);return;}
    if(!this.acceptSteer(message))return;
    try{await this.port.rpc('steer',{message:message.content});}
    catch(error){this.error(error.message);}
  }
  async interrupt() {
    const active=this.active;if(!active)return;active.interrupted=true;this.clearApprovals();
    try{await this.port.rpc('abort',{});}catch{if(this.active===active)this.error('Pi could not confirm turn interruption.');}
  }
  notification(message) {
    if(!message||typeof message!=='object')return;
    if(message.type==='extension_ui_request'){this.ask(message);return;}
    // A run Pi starts by itself -- a queued follow-up, a retry -- is shown
    // like any other rather than dropped.
    // An interrupt that came before the run started found nothing to stop:
    // stop the run now that it has.
    if(message.type==='agent_start'&&this.active){this.active.started=true;if(this.active.interrupted)void this.port.rpc('abort',{}).catch(()=>{});return;}
    if(message.type==='agent_start'&&!this.active){const id=`pi-${Date.now()}-${++this.messageCount}`;this.active={id,interrupted:false,started:true};this.emit({type:'turn',id,status:'running'});return;}
    if(message.type==='agent_settled'){if(this.active)this.finish(this.active.interrupted?'interrupted':this.failed?'failed':'completed');this.failed=false;return;}
    if(!this.active)return;
    if(message.type==='message_start'&&message.message?.role==='assistant'){this.assistantId=`pi-assistant-${++this.messageCount}`;return;}
    if(message.type==='message_update'){
      const delta=message.assistantMessageEvent;
      if(delta?.type==='text_delta'&&typeof delta.delta==='string'){this.assistantId??=`pi-assistant-${++this.messageCount}`;this.emit({type:'message',id:this.assistantId,role:'assistant',text:clip(delta.delta),delta:true});}
      return;
    }
    if(message.type==='message_end'&&message.message?.role==='assistant'){
      const body=message.message,text=textOf(body.content);
      if(text){this.assistantId??=`pi-assistant-${++this.messageCount}`;this.emit({type:'message',id:this.assistantId,role:'assistant',text:clip(text),delta:false});}
      const assistantId=this.assistantId;
      this.assistantId=undefined;
      const usage=body.usage;
      if(usage&&typeof usage==='object'){
        const context=[usage.input,usage.cacheRead,usage.cacheWrite].reduce((sum,value)=>sum+(Number.isSafeInteger(value)?value:0),0);
        this.usage({input_tokens:usage.input,output_tokens:usage.output},context||undefined,this.window);
        const count=value=>Number.isSafeInteger(value)&&value>=0?value:0;
        const cached=count(usage.cacheRead),input=count(usage.input)+cached+count(usage.cacheWrite),output=count(usage.output);
        if(assistantId&&(input||output))this.emit({type:'usage',message_id:assistantId,turn:{input,output,cached}});
      }
      if(body.stopReason==='error'){this.failed=true;this.error(clip(typeof body.errorMessage==='string'&&body.errorMessage?body.errorMessage:'Pi could not complete the turn.',2048));}
      return;
    }
    if(message.type==='tool_execution_start'||message.type==='tool_execution_update'||message.type==='tool_execution_end'){
      const id=message.toolCallId;if(typeof id!=='string'||!id)return;
      const name=clip(typeof message.toolName==='string'?message.toolName:'Tool',256);
      const output=message.type==='tool_execution_end'?message.result:message.partialResult;
      const text=clip([JSON.stringify(message.args??{}),textOf(output?.content)].filter(Boolean).join('\n'));
      this.emit({type:'tool',id,name,status:message.type==='tool_execution_end'?(message.isError?'failed':'completed'):'running',text});
      return;
    }
    if(message.type==='auto_retry_end'&&message.success===false){this.failed=true;this.error(clip(String(message.finalError??'Pi gave up after retrying.'),2048));}
  }
  /**
   * An extension asking the person something. Only the dialogs need an
   * answer; notices and status lines are Pi's own display and are left out.
   */
  ask(message) {
    const id=message.id;if(typeof id!=='string'||!id||id.length>256)return;
    const respond=fields=>{try{this.port.send({type:'extension_ui_response',id,...fields});}catch{/* Pi has gone. */}};
    const title=clip(typeof message.title==='string'?message.title:'Pi needs your input',512);
    if(message.method==='confirm'){
      this.approval(id,{title,text:clip(typeof message.message==='string'?message.message:''),choices:['accept','decline','cancel']},decision=>respond(decision==='cancel'?{cancelled:true}:{confirmed:decision==='accept'}));
      return;
    }
    if(['select','input','editor'].includes(message.method)){
      const options=message.method==='select'&&Array.isArray(message.options)?message.options.filter(option=>typeof option==='string').slice(0,32):[];
      this.approval(id,{title,text:'',choices:['accept','cancel'],questions:[{id:'answer',header:'',question:title,options:options.map(label=>({label:clip(label,256),description:''})),isSecret:false,isOther:message.method!=='select',multiSelect:false}]},(decision,answers)=>{
        const answer=answers?.answer?.[0];
        if(decision==='accept'&&typeof answer!=='string')throw Error('Valid answers required');
        respond(decision==='accept'?{value:answer}:{cancelled:true});
      });
    }
  }
}
