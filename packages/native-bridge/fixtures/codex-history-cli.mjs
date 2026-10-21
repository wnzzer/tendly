#!/usr/bin/env node
// Metadata-only fake native CLI. Only used with temporary config/workspace roots.
import { createInterface } from 'node:readline';
import { appendFileSync } from 'node:fs';
import assert from 'node:assert/strict';

assert.deepEqual(process.argv.slice(2), ['app-server']);
assert.equal(process.env.TENDLY_SECRET_TEST_TOKEN, undefined);
assert.equal(process.env.TENDLY_TOKEN, undefined);
let page = 0;
for await (const line of createInterface({ input: process.stdin })) {
  const request = JSON.parse(line);
  appendFileSync(process.env.TENDLY_HISTORY_TEST_LOG, request.method + '\n');
  if (request.method === 'initialized') continue;
  let result;
  if (request.method === 'initialize') result = { userAgent: 'fixture' };
  else if(request.method==='thread/read') {
    assert.equal(request.params.threadId,'history-1');
    assert.equal(request.params.includeTurns,true);
    result={thread:{cwd:process.cwd(),turns:[{items:[
      {type:'userMessage',id:'u',content:[{type:'text',text:'历史 🦊'}]},
      {type:'agentMessage',id:'a',text:'Loaded reply'},
    ]}]}};
  }
  else {
    assert.equal(request.method, 'thread/list');
    assert.deepEqual(request.params.modelProviders, []);
    assert.equal(request.params.useStateDbOnly, true);
    assert.equal(request.params.sortKey, 'updated_at');
    assert.equal(request.params.limit, 100);
    assert.deepEqual(request.params.sourceKinds, ['cli', 'vscode', 'exec', 'appServer', 'unknown']);
    const row = { id: 'history-1', name: '修复 🦊 布局', cwd: request.params.cwd, updatedAt: 1700000000 };
    result = {
      data: [row, { ...row, id: 'foreign', cwd: '/foreign' }, { id: 'unscoped', updatedAt: 1700000000 }],
      nextCursor: page++ === 0 || process.env.TENDLY_HISTORY_TEST_REPEAT === '1' ? 'next' : null,
    };
  }
  const encoded = Buffer.from(JSON.stringify({ id: request.id, result }) + '\n');
  const unicode = encoded.indexOf(Buffer.from('🦊'));
  if (unicode >= 0) {
    process.stdout.write(encoded.subarray(0, unicode + 1));
    await new Promise(resolve => setTimeout(resolve, 5));
    process.stdout.write(encoded.subarray(unicode + 1));
  } else process.stdout.write(encoded);
}
