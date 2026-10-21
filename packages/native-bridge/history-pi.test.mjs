import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { piHistory } from './history-pi.mjs';

test("a workspace's Pi sessions are listed by their first prompt, and other directories' are not", async () => {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'tendly-pi-history-')));
  try {
    const work = join(root, 'work'), other = join(root, 'other'), home = join(root, 'home');
    await mkdir(work); await mkdir(other);
    const write = async (cwd, id, prompt) => {
      // Pi names the directory after the cwd; the listing reads every one, so
      // any name will do, and a Windows path would not make a valid one.
      const directory = join(home, 'sessions', `--${id}--`);
      await mkdir(directory, { recursive: true });
      await writeFile(join(directory, `2026_${id}.jsonl`), [
        { type: 'session', version: 3, id, timestamp: '2026-10-09T01:00:00.000Z', cwd },
        { type: 'message', id: 'a1', parentId: null, timestamp: '2026-10-09T01:00:01.000Z', message: { role: 'user', content: [{ type: 'text', text: prompt }] } },
      ].map(entry => JSON.stringify(entry)).join('\n') + '\n');
    };
    await write(work, 'pi-one', 'Fix the login form');
    await write(other, 'pi-two', 'Somewhere else');
    const { items, truncated } = await piHistory(work, home);
    assert.equal(truncated, false);
    assert.deepEqual(items.map(item => [item.id, item.provider, item.title, item.cwd]), [['pi-one', 'pi', 'Fix the login form', work]]);
    assert.deepEqual(await piHistory(work, join(root, 'missing')), { items: [], truncated: false });
    const transcript=await piHistory(work,home,undefined,'pi-one');
    assert.equal(transcript.messages[0].content[0].text,'Fix the login form');
    await assert.rejects(piHistory(work,home,undefined,'pi-two'),/not found/);

  } finally { await rm(root, { recursive: true, force: true }); }
});
