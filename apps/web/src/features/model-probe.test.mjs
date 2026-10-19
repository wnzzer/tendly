import test from 'node:test';
import assert from 'node:assert/strict';
import { PROBE_APIS, PROBE_BATCH, apisUsed, checkTally, chooseApi, dropBroken, formatLatency, isBroken, isInconclusive, keepWorking, matchesCheckFilter, mergeChecks, needsCheck, otherKeyHeader, probeAnswer, probeBatches, probeRun, routeSuspicion } from './model-probe.ts';

const result = (model, status, latency_ms, api = 'anthropic-messages') => ({ model, status, api, ...(latency_ms === undefined ? {} : { latency_ms }) });

test('a batch asks for each model once, in the order given, in pieces the server accepts', () => {
  assert.deepEqual(probeBatches(['a', 'b', 'c', 'd', 'e'], 2), [['a', 'b'], ['c', 'd'], ['e']]);
  assert.deepEqual(probeBatches(['a', 'a', ' ', 'b']), [['a', 'b']]);
  assert.deepEqual(probeBatches([]), []);
  // Never more than the server's own cap, whatever the caller asks for.
  assert.ok(PROBE_BATCH > 0 && PROBE_BATCH <= 24);
  assert.deepEqual(probeBatches(['a', 'b', 'c'], 0), [['a'], ['b'], ['c']], 'a nonsense size still moves');
});

test('a repeated check replaces the earlier answer rather than piling up', () => {
  const first = mergeChecks({}, [result('a', 'failed'), result('b', 'ok')]);
  assert.deepEqual(Object.keys(first).sort(), ['a', 'b']);
  const second = mergeChecks(first, [result('a', 'ok')]);
  assert.equal(second.a.status, 'ok');
  assert.equal(second.b.status, 'ok');
  // A malformed row is ignored: one bad answer must not erase a good one.
  assert.deepEqual(mergeChecks(first, [{ model: '', status: 'ok' }]), { a: first.a, b: first.b });
});

test('only a missing model or a plain failure proves a model is broken', () => {
  // A rate limit, a timeout and a refused key say nothing about the model;
  // pruning on them would delete one that works.
  assert.equal(isBroken('not_found'), true);
  assert.equal(isBroken('failed'), true);
  for (const status of ['ok', 'unauthorized', 'rate_limited', 'timeout', undefined]) {
    assert.equal(isBroken(status), false, String(status));
  }
  for (const status of ['unauthorized', 'rate_limited', 'timeout']) {
    assert.equal(isInconclusive(status), true, status);
  }
  assert.equal(isInconclusive('failed'), false);
  assert.equal(isInconclusive(undefined), false);
});

test('the tally counts answers, not models', () => {
  const checks = mergeChecks({}, [
    result('a', 'ok'), result('b', 'ok'), result('c', 'not_found'),
    result('d', 'timeout'), result('e', 'unauthorized'),
  ]);
  assert.deepEqual(checkTally(checks), { tested: 5, ok: 2, broken: 1, inconclusive: 2 });
  assert.deepEqual(checkTally({}), { tested: 0, ok: 0, broken: 0, inconclusive: 0 });
});

test('pruning keeps the chosen order and leaves unchecked models alone', () => {
  const checks = mergeChecks({}, [result('a', 'ok'), result('b', 'failed'), result('c', 'timeout')]);
  assert.deepEqual(keepWorking(['a', 'b', 'c'], checks), ['a']);
  // Unchecked and inconclusive models stay: nothing proved them broken.
  assert.deepEqual(dropBroken(['a', 'b', 'c', 'd'], checks), ['a', 'c', 'd']);
  assert.deepEqual(keepWorking(['a', 'b'], {}), [], 'nothing checked, nothing kept');
});

test('the chips filter by outcome and by what is chosen', () => {
  const checks = mergeChecks({}, [result('a', 'ok'), result('b', 'not_found'), result('c', 'rate_limited')]);
  const chosen = new Set(['a', 'd']);
  const shown = filter => ['a', 'b', 'c', 'd'].filter(model => matchesCheckFilter(model, filter, checks, chosen));
  assert.deepEqual(shown('all'), ['a', 'b', 'c', 'd']);
  assert.deepEqual(shown('chosen'), ['a', 'd']);
  assert.deepEqual(shown('ok'), ['a']);
  assert.deepEqual(shown('broken'), ['b'], 'a rate limit is not a broken model');
  assert.deepEqual(shown('untested'), ['d']);
  assert.deepEqual(shown('inconclusive'), ['c']);
});

test('a model is asked again only when the last answer said nothing about it', () => {
  // Working and missing models keep their answer: re-asking a working model
  // costs money, and re-asking a missing one will not make it appear.
  assert.equal(needsCheck('ok'), false);
  assert.equal(needsCheck('not_found'), false);
  assert.equal(needsCheck('failed'), false);
  // A timeout, a rate limit or a refused key described the moment, not the model.
  assert.equal(needsCheck('timeout'), true);
  assert.equal(needsCheck('rate_limited'), true);
  assert.equal(needsCheck('unauthorized'), true);
  assert.equal(needsCheck(undefined), true, 'never asked');
});

test('latency reads in milliseconds, then in seconds', () => {
  assert.equal(formatLatency(0), '0 ms');
  assert.equal(formatLatency(820), '820 ms');
  assert.equal(formatLatency(999), '999 ms');
  assert.equal(formatLatency(1500), '1.5 s');
  assert.equal(formatLatency(undefined), '');
  assert.equal(formatLatency(Number.NaN), '');
});

test('one press checks a bounded number of models and says how many are left', () => {
  const many = Array.from({ length: 30 }, (_, index) => `m${index}`);
  const run = probeRun(many, {});
  assert.equal(run.models.length, 24, 'one press is one bill the person agreed to');
  assert.equal(run.skipped, 6, 'the rest is reported, not dropped');
  // A second press continues where the first stopped.
  const done = Object.fromEntries(run.models.map(model => [model, result(model, 'ok')]));
  assert.deepEqual(probeRun(many, done).models, many.slice(24));
  // Models that already answered are never asked twice by a press...
  assert.deepEqual(probeRun(['a', 'b'], { a: result('a', 'not_found') }).models, ['b']);
  // ...except by the one-row retry, where asking twice is the whole point.
  assert.deepEqual(probeRun(['a'], { a: result('a', 'failed') }, 24, true).models, ['a']);
});

test('a check names the protocol it used, and the panel reports which ones ran', () => {
  // The same model can answer on one API and fail on another, so an answer
  // without its protocol would be ambiguous.
  const results = [
    result('a', 'ok', 800, 'anthropic-messages'),
    result('b', 'ok', 900, 'anthropic-messages'),
    result('c', 'not_found', undefined, 'openai-responses'),
  ];
  assert.deepEqual(apisUsed(results), ['anthropic-messages', 'openai-responses']);
  assert.deepEqual(apisUsed([]), []);
  // Every protocol a check can speak has a name to show for it.
  for (const api of ['anthropic-messages', 'openai-responses', 'openai-completions', 'google-generative-ai']) {
    assert.ok(PROBE_APIS[api]?.label && PROBE_APIS[api]?.short, api);
  }
});

test('a batch of 404s points at the route, not at every model', () => {
  // One stale model is the catalogue's problem; every model failing the same
  // way is the endpoint's, and a single-protocol client cannot tell them apart
  // by itself.
  const all404 = [result('a', 'not_found'), result('b', 'not_found'), result('c', 'not_found')];
  assert.equal(routeSuspicion(all404), true);
  // One real answer mixed in means the route works and the 404s are genuine.
  assert.equal(routeSuspicion([result('a', 'not_found'), result('b', 'ok')]), false);
  // A refused key is its own, louder problem; it must not be read as a route.
  assert.equal(routeSuspicion([result('a', 'unauthorized'), result('b', 'not_found')]), false);
  // A single model cannot be a pattern.
  assert.equal(routeSuspicion([result('a', 'not_found')]), false);
  assert.equal(routeSuspicion([]), false);
});

test('the protocol kept is the named one if it worked, then the guess, then the first that worked', () => {
  const order = ['openai-completions', 'openai-responses', 'anthropic-messages', 'google-generative-ai'];
  const tried = statuses => order.map((api, index) => result('m', statuses[index], undefined, api));
  // Responses and Messages both run: the profile's own choice stays.
  assert.equal(chooseApi(tried(['not_found', 'ok', 'ok', 'failed']), order, 'anthropic-messages'), 'anthropic-messages');
  // The named one failed: the URL's guess comes next, then the client's order.
  assert.equal(chooseApi(tried(['not_found', 'ok', 'ok', 'failed']), order, 'openai-completions', 'anthropic-messages'), 'anthropic-messages');
  assert.equal(chooseApi(tried(['not_found', 'ok', 'ok', 'failed']), order), 'openai-responses');
  // A refused key or a timeout proves nothing: the profile is not switched.
  assert.equal(chooseApi(tried(['unauthorized', 'timeout', 'rate_limited', 'not_found']), order, 'openai-completions'), undefined);
});

test('an answer reads the same in the panel and in the form check', () => {
  const t = (text, values = {}) => text.replace(/\{(\w+)\}/g, (_, key) => String(values[key]));
  assert.deepEqual(probeAnswer(result('m', 'ok', 820), t), { kind: 'ok', text: 'Works · 820 ms' });
  assert.deepEqual(probeAnswer({ ...result('m', 'not_found'), detail: 'The endpoint has no such model' }, t), { kind: 'broken', text: 'Model not found', title: 'The endpoint has no such model' });
  assert.equal(probeAnswer(result('m', 'unauthorized'), t).kind, 'warn');
});

test('a key refused everywhere is tried once in the other common header, never instead of an endpoint\'s own', () => {
  assert.equal(otherKeyHeader('', 'anthropic-messages'), 'bearer');
  assert.equal(otherKeyHeader('', 'openai-responses'), 'x-api-key');
  assert.equal(otherKeyHeader('', 'google-generative-ai'), 'bearer');
  assert.equal(otherKeyHeader('bearer', 'anthropic-messages'), 'x-api-key');
  assert.equal(otherKeyHeader('x-api-key', 'openai-completions'), 'bearer');
  assert.equal(otherKeyHeader('custom', 'openai-completions'), undefined);
});
