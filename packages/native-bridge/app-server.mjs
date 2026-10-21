// The Codex app-server over stdio, for the bridges that only ask it questions:
// account management, history and file search. Requests are matched to their
// answers by id, within the same bounded JSONL reader the chat bridge uses.
//
// Nothing the server writes to stderr is kept: it can carry configuration and
// authentication details, and none of these bridges may reflect it.
import { jsonLines } from './json-lines.mjs';
import { spawnNative } from './native-spawn.mjs';

/** The server answered with a JSON-RPC error; `code` is its own. */
export class AppServerError extends Error {
  constructor(code) { super('Codex app-server rejected the request.'); this.code = code; }
}
/** Why a connection that never started ended: the program could not be launched. */
export const NOT_STARTED = 'Codex app-server could not start.';
/** The request was cancelled by its caller's signal. */
export class AppServerCancelled extends Error {
  constructor() { super('Codex app-server request was cancelled.'); }
}
/** No answer arrived within the request's timeout. */
export class AppServerTimeout extends Error {
  constructor() { super('Codex app-server request timed out.'); }
}

/**
 * @typedef {{ id?: string | number, method?: string, params?: any, result?: any, error?: { code?: number, message?: string } }} RpcMessage
 * @typedef {object} AppServerOptions
 * @property {string} [cwd]
 * @property {NodeJS.ProcessEnv} [env]
 * @property {number} [timeout] milliseconds a request may wait for its answer
 * @property {number} [maxTotal] bytes the server may write in all
 * @property {number} [maxLine] bytes one message may take
 * @property {(message: RpcMessage) => void} [onNotification]
 * @property {(reason: string) => void} [onClose]
 * @property {typeof spawnNative} [spawnProcess]
 * @typedef {{ signal?: AbortSignal, timeout?: number }} RequestOptions
 */

export class AppServerClient {
  /**
   * `onNotification` receives messages the server sends unasked; `onClose`
   * runs once, with the reason, when the connection ends for any reason. A
   * request the server sends back (to host a token, say) is refused: no bridge
   * offers one.
   * @param {string} program
   * @param {string[]} args
   * @param {AppServerOptions} [options]
   */
  constructor(program, args, { cwd, env, timeout = 20_000, maxTotal = 4 * 1024 * 1024, maxLine = 1024 * 1024, onNotification = () => {}, onClose = () => {}, spawnProcess = spawnNative } = {}) {
    /** @type {Map<string | number, { resolve: (value: any) => void, reject: (error: Error) => void }>} */
    this.pending = new Map(); this.sequence = 0; this.closed = false; this.timeout = timeout;
    this.onNotification = onNotification; this.onClose = onClose;
    this.child = spawnProcess(program, args, { cwd, env, stdio: ['pipe', 'pipe', 'pipe'] });
    this.child.stderr.on('data', () => {});
    this.child.stdin.on('error', () => this.fail('Codex app-server input closed.'));
    this.child.once('error', () => this.fail(NOT_STARTED));
    this.child.once('exit', () => this.fail('Codex app-server exited.'));
    this.stopLines = jsonLines(this.child.stdout, message => this.receive(message), reason => { this.fail(reason); this.child.kill(); }, { maxLine, maxTotal, skipInvalid: true });
  }
  /** @param {RpcMessage} message */
  receive(message) {
    if (message?.id === undefined) { if (message?.method) this.onNotification(message); return; }
    if (message.method) { this.write({ id: message.id, error: { code: -32601, message: 'Not supported by Tendly' } }); return; }
    const pending = this.pending.get(message.id);
    if (pending) message.error ? pending.reject(new AppServerError(message.error.code)) : pending.resolve(message.result);
  }
  /** End the connection: every waiting request fails with `reason`. @param {string} reason */
  fail(reason) {
    if (this.closed) return;
    this.closed = true; this.stopLines?.();
    for (const pending of [...this.pending.values()]) pending.reject(new Error(reason));
    this.pending.clear();
    this.onClose(reason);
  }
  /** @param {RpcMessage} message */
  write(message) {
    if (!this.child.stdin.writable) return;
    this.child.stdin.write(JSON.stringify(message) + '\n', error => { if (error) this.fail('Codex app-server input closed.'); });
  }
  /**
   * @param {string} method
   * @param {object} [params]
   * @param {RequestOptions} [options]
   * @returns {Promise<any>}
   */
  request(method, params = {}, { signal, timeout = this.timeout } = {}) {
    if (signal?.aborted) return Promise.reject(new AppServerCancelled());
    if (this.closed) return Promise.reject(new Error('Codex app-server is closed.'));
    return new Promise((resolve, reject) => {
      const id = ++this.sequence;
      const settle = (callback, value) => { clearTimeout(timer); signal?.removeEventListener('abort', abort); this.pending.delete(id); callback(value); };
      const abort = () => settle(reject, new AppServerCancelled());
      const timer = setTimeout(() => settle(reject, new AppServerTimeout()), timeout);
      this.pending.set(id, { resolve: value => settle(resolve, value), reject: error => settle(reject, error) });
      signal?.addEventListener('abort', abort, { once: true });
      this.write({ id, method, params });
    });
  }
  /** @param {string} method @param {object} [params] */
  notify(method, params = {}) { this.write({ method, params }); }
  /**
   * The handshake every app-server connection starts with.
   * @param {{ name: string, title?: string, version: string }} clientInfo
   * @param {RequestOptions} [options]
   */
  async initialize(clientInfo, options) {
    await this.request('initialize', { clientInfo }, options);
    this.notify('initialized');
  }
  async close() {
    this.fail('Codex app-server is closed.');
    this.child.stdin.end(); this.child.kill('SIGTERM');
    await new Promise(/** @param {(value?: unknown) => void} resolve */ resolve => {
      if (this.child.exitCode !== null || this.child.signalCode) { resolve(); return; }
      const timer = setTimeout(() => { this.child.kill('SIGKILL'); resolve(); }, 1000);
      this.child.once('exit', () => { clearTimeout(timer); resolve(); });
    });
  }
}
