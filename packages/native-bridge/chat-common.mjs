import { resolve, sep } from 'node:path';
import { spawnNative as spawn } from './native-spawn.mjs';
import { randomUUID, createHash } from 'node:crypto';
import { StringDecoder } from 'node:string_decoder';

/**
 * What a native request reaches for, so the card can say it plainly: the kind
 * of access, and any path that lies outside the session's working directory.
 * Paths are only reported, never resolved through the file system.
 *
 * @param {string|undefined} cwd
 * @param {{ access?: string, paths?: unknown[], command?: string }} [request]
 * @returns {{ access?: string, outside?: string[], command?: string } | undefined}
 */
export function approvalScope(cwd, request = {}) {
  const { access, paths = [], command } = request;
  const root=typeof cwd==='string'&&cwd?resolve(cwd):undefined;
  const outside=!root?[]:[...new Set(paths.filter(/** @returns {path is string} */ path=>typeof path==='string'&&path.length>0&&path.length<=1024&&!path.includes('\0')).map(path=>resolve(root,path)))]
    .filter(path=>path!==root&&!path.startsWith(root.endsWith(sep)?root:root+sep)).slice(0,8);
  /** @type {{ access?: string, outside?: string[], command?: string }} */
  const scope={};
  if(typeof access==='string'&&['read','write','execute','network'].includes(access))scope.access=access;
  if(outside.length)scope.outside=outside;
  if(typeof command==='string'&&command.trim())scope.command=command.length>600?command.slice(0,599)+'…':command;
  return Object.keys(scope).length?scope:undefined;
}

export const MAX_INPUT_LINE = 512 * 1024;
// One line from the client. Codex reports a generated image inline as base64,
// several megabytes for one picture, so this is sized for images, not text.
export const MAX_NATIVE_LINE = 64 * 1024 * 1024;
// Everything the client writes over the session's life. Each line is read and
// let go, so this only stops a runaway stream; an image-heavy session is not one.
export const MAX_NATIVE_TOTAL = 2 * 1024 * 1024 * 1024;
// What this bridge may queue for the client to read.
export const MAX_NATIVE_INPUT = 4 * 1024 * 1024;
export const MAX_TEXT = 64 * 1024;
export const MAX_EVENT_LINE = 192 * 1024;
export const nativeId = value => typeof value === 'string' && /^[a-zA-Z0-9][a-zA-Z0-9_-]{0,127}$/.test(value);
export function clip(value, limit = MAX_TEXT) {
  if (typeof value !== 'string') return '';
  const cleaned = value.replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, '').replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, '');
  const bytes = Buffer.from(cleaned);
  return bytes.length <= limit ? cleaned : bytes.subarray(0, limit).toString('utf8') + '\n[truncated]';
}
export function redactEvents(environment, output) {
  const secrets = Object.entries(environment).filter(([key, value]) => /TOKEN|SECRET|PASSWORD|PRIVATE_KEY|API_KEY|AUTHORIZATION/i.test(key) && typeof value === 'string' && value.length >= 4).map(([, value]) => value).sort((a,b) => b.length-a.length);
  return event => {
    const scrub = value => typeof value === 'string' ? secrets.reduce((text, secret) => text.split(secret).join('[redacted]'), value) : Array.isArray(value) ? value.map(scrub) : value && typeof value === 'object' ? Object.fromEntries(Object.entries(value).map(([key,item]) => [key,scrub(item)])) : value;
    output(scrub(event));
  };
}

/**
 * Bounded UTF-8 JSONL, including a fixed deadline for incomplete lines.
 * `skipInvalid` passes over a line that is not JSON instead of failing the
 * stream, for protocols that only care about the answers they asked for.
 */
export function jsonLines(readable, onMessage, onFailure, { maxLine = MAX_NATIVE_LINE, maxTotal = MAX_NATIVE_TOTAL, lineTimeout = 10_000, skipInvalid = false } = {}) {
  // An incomplete line is kept as the pieces it came in, with their size, so a
  // long one is looked through once as it arrives rather than with every chunk.
  const decoder = new StringDecoder('utf8'); let pieces = [], pending = 0, total = 0, timer, stopped = false;
  const clear = () => { clearTimeout(timer); timer = undefined; };
  const fail = message => { if (!stopped) { stopped = true; clear(); onFailure(message); } };
  const data = chunk => {
    if (stopped) return;
    total += chunk.length; if (total > maxTotal) { fail('Structured stream exceeded its size limit.'); return; }
    let text = decoder.write(chunk), index;
    while ((index = text.indexOf('\n')) !== -1) {
      clear(); const head = text.slice(0,index); text = text.slice(index+1);
      const size = pending + Buffer.byteLength(head), line = pieces.length ? pieces.join('') + head : head;
      pieces = []; pending = 0;
      if (size > maxLine) { fail('Structured message exceeded its size limit.'); return; }
      if (!line.trim()) continue;
      let message;
      try { message = JSON.parse(line); } catch { if (skipInvalid) continue; fail('Invalid structured message received.'); return; }
      try { onMessage(message); } catch { fail('Invalid structured message received.'); return; }
      if (stopped) return;
    }
    if (text) { pieces.push(text); pending += Buffer.byteLength(text); }
    if (pending > maxLine) { fail('Structured message exceeded its size limit.'); return; }
    if (pending && !timer) timer = setTimeout(() => fail('Incomplete structured message timed out.'), lineTimeout);
  };
  const end = () => { const rest = pieces.join('') + decoder.end(); if (rest.trim() && !stopped) fail('Structured stream ended mid-message.'); clear(); };
  readable.on('data',data); readable.on('end',end);
  return () => { stopped = true; clear(); readable.off('data',data); readable.off('end',end); };
}

/**
 * JSON-RPC, the request envelope unless a client's chat names its own.
 * @type {(id: string, method: string, params: object) => object}
 */
const jsonRpcRequest = (id, method, params) => ({ id, method, params });

/**
 * Which pending request a message answers, and how: a JSON-RPC result or a
 * Claude `control_response`, unless a client's chat reads its own. Anything
 * else is not an answer and goes on as a notification.
 * @type {(message: any) => { id: any, refused?: string|true, result?: any } | undefined}
 */
const knownReply = message => {
  if (message?.type === 'control_response') {
    const response = message.response ?? {};
    return { id: response.request_id, ...(response.subtype === 'error' ? { refused: response.error ?? true } : { result: response.response ?? {} }) };
  }
  if (message?.method || message?.id === undefined) return undefined;
  return { id: message.id, ...(message.error ? { refused: message.error?.message ?? message.error } : { result: message.result }) };
};

export class NativeProcess {
  /**
   * `frame` wraps one request in the client's wire envelope. Replies of either
   * known shape -- a JSON-RPC result or a Claude `control_response` -- are
   * matched back to it below.
   */
  constructor(program, args, cwd, onMessage, onFailure, onExit, { spawnProcess = spawn, rpcTimeout = 30_000, frame = jsonRpcRequest, reply = knownReply, clientName = 'Claude Code' } = {}) {
    this.pending = new Map(); this.sequence = 0; this.closed = false; this.rpcTimeout = rpcTimeout; this.frame = frame;
    this.failure = message => { if (!this.closed) onFailure(message); };
    this.child = spawnProcess(program,args,{ cwd, env: process.env, stdio:['pipe','pipe','pipe'], shell:false });
    this.stopLines = jsonLines(this.child.stdout, message => {
      const answer = reply(message);
      const pending = answer && this.pending.get(answer.id);
      if (pending) {
        this.pending.delete(answer.id); clearTimeout(pending.timer);
        if (answer.refused !== undefined) {
          // The client says exactly why it refused. Dropping that left the user
          // with a generic message and nothing to act on, so its own wording is
          // carried through — clipped, and already redacted on the way out.
          const stated = answer.refused;
          const detail = typeof stated === 'string' && stated.trim() ? clip(stated.trim(), 400) : undefined;
          pending.reject(new Error(detail
            ? `${clientName} refused: ${detail}`
            : 'Native client rejected a control request. Check client settings and version.'));
        }
        else pending.resolve(answer.result);
      } else onMessage(message);
    }, this.failure);
    let diagnosticBytes = 0;
    this.child.stderr.on('data',chunk => { diagnosticBytes += chunk.length; if (diagnosticBytes > 16 * 1024 * 1024) this.failure('Native diagnostic output exceeded its size limit.'); });
    this.child.stdin.on('error',() => this.failure('Native client input closed.'));
    this.child.once('error',() => this.failure('Could not launch the configured native client.'));
    this.child.once('exit',() => {
      this.stopLines(); this.rejectPending(); if (!this.closed) onExit();
    });
  }
  rejectPending() { for (const pending of this.pending.values()) { clearTimeout(pending.timer); pending.reject(new Error('Native client closed.')); } this.pending.clear(); }
  send(message) {
    if (this.closed || !this.child.stdin.writable) throw new Error('Native client input is unavailable.');
    const line = JSON.stringify(message) + '\n';
    if (Buffer.byteLength(line) > MAX_NATIVE_INPUT || this.child.stdin.writableLength > MAX_NATIVE_INPUT) throw new Error('Native input exceeded its buffer limit.');
    this.child.stdin.write(line);
  }
  /**
   * `optional` is for a capability probe: a client that does not implement the
   * method may simply never answer, and that must not look like a broken
   * connection. Such a call fails quietly and changes nothing.
   */
  rpc(method, params, optional = false) {
    const id = `tendly-${++this.sequence}`;
    // A model list is asked for in the background and nothing waits on it,
    // while a ChatGPT-signed Codex can take five seconds to fetch one; cut off
    // at the probe's limit, the composer lost its model, depth and permission chips.
    const listing = ['model/list','get_available_models'].includes(method);
    return new Promise((resolve,reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);reject(new Error('Native control request timed out.'));
        // A timed-out start may have executed. Do not reopen the send gate and guess/replay it.
        if(!optional&&!['interrupt','turn/interrupt','abort'].includes(method))this.failure('Native control request timed out; the structured connection was stopped.');
      },optional&&!listing?Math.min(this.rpcTimeout,4000):this.rpcTimeout);
      this.pending.set(id,{resolve,reject,timer});
      try { this.send(this.frame(id, method, params)); }
      catch(error) { clearTimeout(timer);this.pending.delete(id);reject(error); }
    });
  }
  async close() {
    if (this.closed) return;
    this.closed = true; this.stopLines(); this.rejectPending();
    this.child.stdin.end(); this.child.kill('SIGTERM');
    await new Promise(resolve => {
      if (this.child.exitCode !== null || this.child.signalCode) { resolve();return; }
      const timer = setTimeout(() => { this.child.kill('SIGKILL');resolve(); },1500);
      this.child.once('exit',() => {clearTimeout(timer);resolve();});
    });
  }
}

/**
 * The turn a chat is running. `early`, `nativeTurn` and `interruptSent` are
 * Codex's: steers that arrived before its turn had an id, that id, and whether
 * an interrupt already went out.
 * @typedef {{ id: string, interrupted: boolean, early?: any[], nativeTurn?: any, interruptSent?: boolean, started?: boolean, turnUsage?: { input: number, output: number, cached: number } }} Turn
 */

export class ChatBase {
  constructor(job, emit, options = {}) {
    this.job=job;this.emit=emit;this.options=options;this.seen=new Map();this.approvals=new Map();this.ready=false;this.closed=false;this.deferred=[];
    /** @type {Turn | undefined} */ this.active=undefined;
    // Set by each client's chat once it is known.
    /** @type {NativeProcess | undefined} */ this.port=undefined;
    /** @type {string | undefined} */ this.permissionMode=undefined;
  }
  /**
   * The permission modes the client accepts, when it says. Each client's chat
   * overrides this getter; assigning the property instead would throw there.
   * @returns {string[] | undefined}
   */
  get permissionModes() { return undefined; }
  error(message) { this.emit({type:'error',message:clip(message,2048)}); }
  begin(message) {
    if (!this.ready || this.closed) { this.error('The native client is not ready.');return; }
    const digest=createHash('sha256').update(message.content).digest('hex');
    if (this.seen.has(message.id)) { if(this.seen.get(message.id)!==digest)this.error('Message ID was already used for different content.');return; }
    if(this.active){this.error('A turn is already running. Interrupt it or wait before sending another message.');return;}
    if(this.seen.size>=4096){this.error('This bridge reached its message limit. Reopen the saved native session.');this.emit({type:'turn',id:message.id,status:'failed'});return;}
    this.seen.set(message.id,digest);this.active={id:message.id,interrupted:false,early:[]};
    this.emit({type:'message',id:message.id,role:'user',text:clip(message.content),delta:false});
    this.emit({type:'turn',id:message.id,status:'running'});return this.active;
  }
  /**
   * A message written while a turn runs, for that turn rather than the next.
   * Returns false when it was already seen or cannot be taken; the caller
   * runs it as an ordinary message when no turn is active any more.
   */
  acceptSteer(message) {
    if (!this.ready || this.closed) { this.error('The native client is not ready.');return false; }
    const digest=createHash('sha256').update(message.content).digest('hex');
    if (this.seen.has(message.id)) { if(this.seen.get(message.id)!==digest)this.error('Message ID was already used for different content.');return false; }
    if(this.seen.size>=4096){this.error('This bridge reached its message limit. Reopen the saved native session.');return false;}
    this.seen.set(message.id,digest);return true;
  }
  finish(status) {
    if(!this.active)return;
    const id=this.active.id;this.active=undefined;
    this.clearApprovals(false);this.emit({type:'turn',id,status});
  }
  // `commands` are the slash commands the native client itself advertised on
  // startup. They are forwarded verbatim so the composer can offer exactly what
  // that client accepts, never a list Tendly invented.
  announce(id,commands) {
    // A client that hands back a different session has started a fresh context
    // of its own — /clear does exactly this. The transcript above it describes
    // a context the client no longer has, so it is not kept as if it did.
    if(nativeId(id)&&this.nativeSessionId&&id!==this.nativeSessionId)this.emit({type:'cleared'});
    if(nativeId(id))this.nativeSessionId=id;
    this.ready=true;
    const names=Array.isArray(commands)
      ? [...new Set(commands.filter(name=>typeof name==='string'&&/^[A-Za-z0-9][\w:-]{0,63}$/.test(name)))].slice(0,400)
      : undefined;
    this.emit({type:'ready',...(this.nativeSessionId?{native_session_id:this.nativeSessionId}:{}),...(names?.length?{commands:names}:{})});
  }
  /**
   * What the client says it can run right now: the models it offers and which
   * one is selected. This is deliberately not a `configuration` event — that one
   * marks a context boundary, and selecting a model must not start a new one.
   * Entries are forwarded from the client's own list; nothing is added.
   */
  settings(models, model, effort) {
    const list=Array.isArray(models)
      ? models.flatMap(entry=>{
          if(!entry||typeof entry!=='object'||typeof entry.id!=='string'||!entry.id||entry.id.length>128)return [];
          // Only levels both clients share. An unfamiliar one drops itself
          // rather than taking the whole list down with it downstream.
          const EFFORTS=['low','medium','high','xhigh','max'];
          const efforts=Array.isArray(entry.efforts)?entry.efforts.filter(value=>EFFORTS.includes(value)).slice(0,16):undefined;
          return [{id:entry.id,name:clip(typeof entry.name==='string'&&entry.name?entry.name:entry.id,128),
            ...(typeof entry.description==='string'&&entry.description?{description:clip(entry.description,256)}:{}),
            // The row the configuration resolves to. A session that has not
            // started yet has no model of its own, and this is what it shows
            // instead of electing the first row or offering no depth at all.
            ...(entry.isDefault===true?{isDefault:true}:{}),
            ...(efforts?.length?{efforts}:{})}];
        }).slice(0,64)
      : undefined;
    const selected=typeof model==='string'&&model&&model.length<=128?model:undefined;
    const depth=typeof effort==='string'&&/^[a-z]{1,16}$/.test(effort)?effort:undefined;
    // How tools are approved is the client's own state, reported alongside the
    // rest of what this session is currently set to.
    const permission=typeof this.permissionMode==='string'&&/^[a-z_]{1,24}$/.test(this.permissionMode)?this.permissionMode:undefined;
    // Which modes exist is the client's answer, not this bridge's: Codex has no
    // equivalent of plan or accept-edits, and offering a control that errors on
    // use would be worse than not offering it.
    const available=Array.isArray(this.permissionModes)&&this.permissionModes.length?this.permissionModes.filter(mode=>/^[a-z_]{1,24}$/.test(mode)).slice(0,8):undefined;
    // The modes alone are worth saying: without them a client whose model list
    // never came had no permission chip either.
    if(!list?.length&&!selected&&!depth&&!permission&&!available?.length)return;
    this.emit({type:'settings',...(selected?{model:selected}:{}),...(depth?{effort:depth}:{}),...(permission?{permission_mode:permission}:{}),...(available?.length?{permission_modes:available}:{}),...(list?.length?{models:list}:{})});
  }
  approval(key, description, answer) {
    if(this.approvals.has(key))return;
    if(this.approvals.size>=32){answer(description.choices.includes('cancel')?'cancel':'decline',{});this.error('Too many pending native approvals.');return;}
    const id=randomUUID();
    const timer=setTimeout(() => {this.resolveApproval(key,description.choices.includes('cancel')?'cancel':'decline',{});this.error('Native approval timed out and was cancelled.');},this.options.approvalTimeout??600_000);
    this.approvals.set(key,{id,answer,timer,choices:description.choices});this.emit({type:'approval',id,...description});
  }
  resolveApproval(key,decision,answers) {
    const pending=this.approvals.get(key);if(!pending)return;
    try { pending.answer(decision,answers); } catch { this.error('Invalid or unavailable native approval response.');return; }
    if(this.approvals.get(key)!==pending)return;
    this.approvals.delete(key);clearTimeout(pending.timer);
    this.emit({type:'approval_resolved',id:pending.id});
  }
  answer(message) {
    const entry=[...this.approvals].find(([,pending])=>pending.id===message.request_id);
    if(!entry){this.error('This approval is no longer pending.');return;}
    if(!entry[1].choices.includes(message.decision)){this.error('This decision is not offered by the native request.');return;}
    this.resolveApproval(entry[0],message.decision,message.answers??{});
  }
  resolveNative(key) { const pending=this.approvals.get(key);if(pending){clearTimeout(pending.timer);this.approvals.delete(key);this.emit({type:'approval_resolved',id:pending.id});} }
  clearApprovals(sendCancellation=true) { for(const key of [...this.approvals.keys()])sendCancellation?this.resolveApproval(key,this.approvals.get(key).choices.includes('cancel')?'cancel':'decline',{}):this.resolveNative(key); }
  async shutdown() { if(this.closed)return;this.closed=true;this.clearApprovals();this.finish('interrupted');await this.port?.close(); }
  fatal(message) { if(this.closed)return;this.error(message);this.finish('failed');void this.shutdown().finally(()=>this.options.onExit?.()); }
  // `window` is the model's own context size as the client reported it. It is
  // only ever forwarded, never assumed, so a share of context can be shown when
  // the client states the limit and is simply omitted when it does not.
  usage(usage,context,window) {
    const values={input_tokens:usage?.inputTokens??usage?.input_tokens,output_tokens:usage?.outputTokens??usage?.output_tokens,context_tokens:context,context_window:window};
    const filtered=Object.fromEntries(Object.entries(values).filter(([key,value])=>Number.isSafeInteger(value)&&(key==='context_window'?value>0:value>=0)));
    if(Object.keys(filtered).length)this.emit({type:'usage',...filtered});
  }
}
